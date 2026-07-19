//! Admin commands: system-settings show/update, promote, request review.
//! Round-trips CLI → AdminClient → API → access_service.

use crate::error::{Result, TemperError};
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::admin::{PromoteAdminRequest, UpdateSettingsRequest};

/// Show settings when no flag is set; otherwise PATCH and render the result.
///
/// The `UpdateSettingsRequest` is built in the `main.rs` dispatch arm and passed
/// by value — no per-field args needed here.
pub async fn settings_remote(
    client: &temper_client::TemperClient,
    req: UpdateSettingsRequest,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let settings = if req.is_empty() {
        client
            .admin()
            .get_settings()
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?
    } else {
        client
            .admin()
            .update_settings(&req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?
    };

    let rendered = crate::format::render(&settings, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Promote a profile to owner on a team (defaults to the gating team).
pub async fn promote_remote(
    client: &temper_client::TemperClient,
    profile: &str,
    team: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;

    // Resolve --team to a UUID when provided; None ⇒ server uses the gating team.
    let team_id = match team {
        Some(t) => Some(crate::actions::cogmap::resolve_team_id(client, t).await?),
        None => None,
    };

    let req = PromoteAdminRequest {
        profile_id,
        team_id,
    };
    let row = client
        .admin()
        .promote(&req)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// List pending join requests.
pub async fn requests_list_remote(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let rows = client
        .admin()
        .list_requests()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&rows, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Approve or reject a join request.
pub async fn requests_review_remote(
    client: &temper_client::TemperClient,
    id: &str,
    approve: bool,
    reject: bool,
    note: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let request_id = uuid::Uuid::parse_str(id)
        .map_err(|e| TemperError::Api(format!("invalid request id '{id}': {e}")))?;

    let decision = match (approve, reject) {
        (true, false) => JoinRequestStatus::Approved,
        (false, true) => JoinRequestStatus::Rejected,
        _ => {
            return Err(TemperError::Api(
                "specify exactly one of --approve or --reject".to_string(),
            ))
        }
    };

    let row = client
        .admin()
        .review_request(request_id, decision, note.map(str::to_owned))
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    let rendered = crate::format::render(&row, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// `temper admin reembed` — trigger a re-embed for a scope of the index (admin only).
///
/// The *trigger*, not the engine: it enqueues embed jobs for chunks whose vector was produced by a
/// model that is no longer the one the server embeds with, and the per-minute drain does the work.
///
/// Nothing is marked dirty and nothing is destroyed — staleness is derived
/// (`embedding IS NULL OR embedded_with IS DISTINCT FROM <current model>`), and the stale vector stays
/// searchable until a fresh one replaces it. So this is idempotent, safe to re-run, and safe to run
/// while the drain is mid-flight.
///
/// Scope it small first. `--dry-run` surveys without enqueuing anything; `--resource` does one;
/// `--context` does one context; `--all` does everything. `--limit` bounds how many resources a single
/// call queues, so "re-embed the index" is a walk, not a leap.
pub async fn reembed_remote(
    client: &temper_client::TemperClient,
    resource: Option<String>,
    context: Option<String>,
    all: bool,
    limit: Option<i32>,
    dry_run: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource_id = match resource.as_deref() {
        Some(r) => Some(
            temper_workflow::operations::parse_ref(r)
                .map_err(|e| TemperError::BadRequest(format!("invalid resource ref {r:?}: {e}")))?,
        ),
        None => None,
    };
    let context_id = match context.as_deref() {
        Some(c) => {
            Some(crate::commands::context_cmd::resolve_context_id_for_read(client, c).await?)
        }
        None => None,
    };

    // Exactly one scope — refuse to guess. "All" must be asked for by name.
    let scopes = [resource_id.is_some(), context_id.is_some(), all]
        .iter()
        .filter(|x| **x)
        .count();
    if scopes != 1 {
        return Err(TemperError::BadRequest(
            "specify exactly one of --resource, --context, or --all".to_string(),
        ));
    }

    let body = temper_core::types::admin::ReembedRequest {
        resource_id: resource_id.map(|r| *r),
        context_id,
        all,
        limit,
        dry_run,
    };
    let summary = client
        .admin()
        .reembed(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&summary, fmt)?;
    println!("{rendered}");
    Ok(())
}

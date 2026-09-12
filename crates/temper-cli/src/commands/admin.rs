//! Admin commands: system-settings show/update, promote, request review.
//! Round-trips CLI → AdminClient → API → access_service.

use crate::error::{Result, TemperError};
use temper_core::types::access_gate::JoinRequestStatus;
use temper_core::types::admin::{AdminLedgerQuery, PromoteAdminRequest, UpdateSettingsRequest};

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

/// Demote a system admin — revoke its governance grant. The manual twin of `promote`; not
/// team-scoped (governance is keyed on the profile alone). Prints `{profile_id} demoted` on success,
/// mirroring the standing acts.
pub async fn demote_remote(client: &temper_client::TemperClient, profile: &str) -> Result<()> {
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;

    client
        .admin()
        .demote(profile_id)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    println!("{profile_id} demoted");
    Ok(())
}

/// Promote a profile to system admin (grants governance + approved standing;
/// the side-effect `owner` row defaults to the gating team).
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

/// The admin standing acts (Task 13): approve / revoke / deactivate / reactivate a principal.
/// Each is a POST that returns `200 OK` with no body, so success is reported as a line rather than
/// a rendered payload.
pub async fn access_remote(
    client: &temper_client::TemperClient,
    action: &crate::cli::AdminAccessAction,
) -> Result<()> {
    use crate::cli::AdminAccessAction;

    let parse = |profile: &str| {
        uuid::Uuid::parse_str(profile)
            .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))
    };
    let admin = client.admin();

    let (profile_id, verb) = match action {
        AdminAccessAction::Approve { profile } => {
            let id = parse(profile)?;
            admin
                .approve_principal(id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            (id, "approved")
        }
        AdminAccessAction::Revoke { profile, reason } => {
            let id = parse(profile)?;
            admin
                .revoke_principal(id, reason)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            (id, "revoked")
        }
        AdminAccessAction::Deactivate { profile } => {
            let id = parse(profile)?;
            admin
                .deactivate_principal(id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            (id, "deactivated")
        }
        AdminAccessAction::Reactivate { profile } => {
            let id = parse(profile)?;
            admin
                .reactivate_principal(id)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            (id, "reactivated")
        }
    };

    println!("{profile_id} {verb}");
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

/// List undecided reconsideration requests (spec D15).
pub async fn reviews_list_remote(
    client: &temper_client::TemperClient,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let rows = client
        .admin()
        .list_reviews()
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&rows, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Record that a reconsideration was handled. Grants nothing — see `AdminReviewsAction` in `cli.rs`.
///
/// Prints the id it closed rather than nothing at all: the endpoint answers `204`, and a command
/// that succeeds silently gives an operator working a queue no way to tell "closed it" from "typed
/// the wrong id and closed something else".
pub async fn reviews_close_remote(
    client: &temper_client::TemperClient,
    id: &str,
    note: Option<&str>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let request_id = uuid::Uuid::parse_str(id)
        .map_err(|e| TemperError::Api(format!("invalid review id '{id}': {e}")))?;

    client
        .admin()
        .close_review(request_id, note.map(str::to_owned))
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;

    let rendered = crate::format::render(
        &serde_json::json!({ "status": "closed", "review": request_id }),
        fmt,
    )?;
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
    // Exactly one scope — refuse to guess, BEFORE any ref resolution: an ambiguous invocation
    // must not spend authenticated round-trips learning it is ambiguous, and a resolution
    // failure must not mask the refusal (the family pattern, same as `reblock_remote`).
    // "All" must be asked for by name.
    let scopes = [resource.is_some(), context.is_some(), all]
        .iter()
        .filter(|x| **x)
        .count();
    if scopes != 1 {
        return Err(TemperError::BadRequest(
            "specify exactly one of --resource, --context, or --all".to_string(),
        ));
    }

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

/// `temper admin reblock` — run one bounded, resumable corpus re-blocking step (admin only for
/// the deployment-wide arm; the resource and context arms ride the caller's own visibility).
///
/// Each candidate whose stored body no longer reproduces its stored chunking is re-blocked
/// under the current chunking policy — ordinary per-resource writes, gated one row at a time
/// by the invoking operator's own write predicates; the batch mints no authority.
///
/// Survey it first: `--dry-run` classifies every candidate without touching anything. Then run
/// without it, then survey again to verify. Exactly one scope — refuse to guess. "All" must be
/// asked for by name. `--limit` bounds how many candidates a single call considers, and
/// `--after-id` resumes a walk from the previous receipt's cursor, so "reblock the corpus" is a
/// walk, not a leap.
#[allow(clippy::too_many_arguments)]
pub async fn reblock_remote(
    client: &temper_client::TemperClient,
    resource: Option<String>,
    context: Option<String>,
    all: bool,
    dry_run: bool,
    limit: Option<i64>,
    after_id: Option<uuid::Uuid>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    // Exactly one scope — refuse to guess, BEFORE any ref resolution: an ambiguous invocation
    // must not spend authenticated round-trips learning it is ambiguous, and a resolution
    // failure must not mask the refusal. "All" must be asked for by name.
    let scopes = [resource.is_some(), context.is_some(), all]
        .iter()
        .filter(|x| **x)
        .count();
    if scopes != 1 {
        return Err(TemperError::BadRequest(
            "specify exactly one of --resource, --context, or --all".to_string(),
        ));
    }

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

    let scope = if let Some(r) = resource_id {
        temper_core::types::reblock::ReblockScope::Resource(*r)
    } else if let Some(c) = context_id {
        temper_core::types::reblock::ReblockScope::Context(c)
    } else {
        // The exclusivity check above guarantees `all` — the deployment-wide arm is reached
        // only by naming it.
        temper_core::types::reblock::ReblockScope::All
    };

    let body = temper_core::types::reblock::ReblockRequest {
        scope,
        dry_run,
        limit,
        after_id,
    };
    let receipt = client
        .admin()
        .reblock(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&receipt, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// Read the admin ledger — "who granted what, to whom, when".
///
/// Exactly one axis. `--subject <kind>:<uuid>` asks what was done TO a thing; `--actor <uuid>`
/// asks what a principal DID. They gate differently (subject reads are checked per act family
/// against that subject; actor reads are self-gating), which is why the server refuses a request
/// naming both rather than picking one — and why clap refuses it here too, so the round-trip is
/// not spent learning that.
///
/// A refusal is a **404**, deliberately: on this surface "you may not read that" and "there is
/// nothing there" are made indistinguishable, because a 403 would confirm the ledger holds
/// something about the subject.
pub async fn ledger_remote(
    client: &temper_client::TemperClient,
    subject: Option<&str>,
    actor: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let mut query = AdminLedgerQuery {
        limit,
        offset,
        ..Default::default()
    };

    match (subject, actor) {
        (Some(_), Some(_)) => {
            // Belt and braces: clap's `conflicts_with` already refuses this. Kept so the rule
            // survives a future caller that builds the args programmatically.
            return Err(TemperError::Api(
                "pass either --subject or --actor, not both".to_string(),
            ));
        }
        (None, None) => {
            return Err(TemperError::Api(
                "pass either --subject <kind>:<uuid> or --actor <uuid>".to_string(),
            ));
        }
        (Some(spec), None) => {
            // Passed through verbatim. The server owns the `<kind>:<uuid>` grammar AND the anchor
            // vocabulary, and reports both with the offending value — a copy here would be a
            // second grammar to keep in step for no gain.
            query.subject = Some(spec.to_string());
        }
        (None, Some(actor)) => {
            query.actor = Some(
                uuid::Uuid::parse_str(actor)
                    .map_err(|e| TemperError::Api(format!("invalid actor id '{actor}': {e}")))?,
            );
        }
    }

    let page = client
        .admin()
        .ledger(&query)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    let rendered = crate::format::render(&page, fmt)?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A client pointed at a dead loopback port: nothing is listening, so any request that
    /// actually leaves the process fails at transport — an error class distinct from the
    /// scope-validation refusals these tests assert on.
    fn dead_client() -> temper_client::TemperClient {
        temper_client::TemperClient::with_token(
            "http://127.0.0.1:9",
            None,
            temper_workflow::operations::Surface::CliCloud,
            "test-token".to_string(),
            std::sync::Arc::new(temper_client::auth::MemoryTokenStore::empty()),
        )
        .expect("loopback URL validates")
    }

    const SCOPE_MESSAGE: &str = "specify exactly one of --resource, --context, or --all";

    /// No scope flag is no scope at all — the command refuses rather than guessing a default.
    /// The deployment-wide arm must be asked for by name, never arrived at by omission.
    #[tokio::test]
    async fn reblock_with_no_scope_flag_errors() {
        let err = reblock_remote(
            &dead_client(),
            None,
            None,
            false,
            false,
            None,
            None,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect_err("no scope flag must error");
        assert!(matches!(err, TemperError::BadRequest(ref m) if m == SCOPE_MESSAGE));
    }

    /// Two scopes at once is ambiguous — refused, not resolved by precedence. The refusal must
    /// come BEFORE any ref resolution: an ambiguous invocation must not spend authenticated
    /// round-trips learning it is ambiguous, and a resolution failure must not mask it. Against
    /// the dead client, a pre-check resolution of the context ref surfaces as a Network error —
    /// exactly what this witness bites on.
    /// FAILS IF: the exclusivity check moves back below the ref resolutions.
    #[tokio::test]
    async fn reblock_with_two_scope_flags_errors() {
        let err = reblock_remote(
            &dead_client(),
            Some("019e84ab-26ba-7560-9d34-c60d74a9fbe2".to_string()),
            Some("@me/temper".to_string()),
            false,
            false,
            None,
            None,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect_err("two scope flags must error");
        assert!(
            matches!(err, TemperError::BadRequest(ref m) if m == SCOPE_MESSAGE),
            "the scope refusal must win over any resolution failure, got: {err}"
        );
    }

    /// Exactly one scope passes validation and proceeds to dispatch. Against the dead
    /// endpoint the dispatch itself fails at transport — asserted POSITIVELY on the
    /// Network error class, which is what distinguishes "validated, then sent" from
    /// "refused" (a negative on the scope message is satisfied vacuously by any future
    /// pre-dispatch refusal).
    #[tokio::test]
    async fn reblock_with_exactly_one_scope_flag_reaches_dispatch() {
        let err = reblock_remote(
            &dead_client(),
            Some("019e84ab-26ba-7560-9d34-c60d74a9fbe2".to_string()),
            None,
            false,
            false,
            None,
            None,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect_err("dead endpoint must fail at transport");
        assert!(
            matches!(err, TemperError::Network(_)),
            "a single scope must pass validation and fail at transport (Network), got: {err}"
        );
    }
}

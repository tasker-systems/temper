//! `temper admin erasure` — the erasure family's operator acts: erase a resource, erase a
//! principal, scrub a resource's block history. System admin only.
//!
//! Each act is surveyed first: `--dry-run` asks the survey door what the act would do and records
//! nothing. Run without it, the act executes — once: the client never replays a write, and an
//! erasure is the last write that should be replayed.
//!
//! The answer is rendered in full, whatever the format: the per-target outcomes, the named
//! remainder, the ledger remainder, the refusal's reason and detail. A completed act and a refused
//! one read differently in two ways: the answer's `status` field, and the exit code — a refusal
//! prints its answer (it is the record of what happened) and then exits non-zero naming the reason,
//! so a script that drove the act cannot mistake a refusal for success.
//!
//! The bearer token comes from the token store, as for every command. Nothing here takes it as an
//! argument or prints it.

use crate::error::{Result, TemperError};
use temper_core::types::erasure::{
    BlockHistoryScrubExecuteResponse, BlockHistoryScrubRequestBody, ErasureExecuteRequest,
    ErasureSurveyRequest, ResourceErasureExecuteRequest, ResourceErasureExecuteResponse,
    ResourceErasureRefusalReason, ResourceErasureSurveyRequest,
};

fn parse_resource(resource: &str) -> Result<uuid::Uuid> {
    let id = temper_workflow::operations::parse_ref(resource)
        .map_err(|e| TemperError::BadRequest(format!("invalid resource ref {resource:?}: {e}")))?;
    Ok(*id)
}

fn print<T: serde::Serialize>(value: &T, fmt: crate::format::OutputFormat) -> Result<()> {
    let rendered = crate::format::render(value, fmt)?;
    println!("{rendered}");
    Ok(())
}

/// The error a refused act exits with, after its answer has been printed.
fn refused(act: &str, reason: ResourceErasureRefusalReason, detail: Option<&str>) -> TemperError {
    let reason = serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{reason:?}"));
    let detail = detail.map(|d| format!(": {d}")).unwrap_or_default();
    TemperError::Conflict(format!(
        "{act} refused ({reason}){detail}. The refusal is recorded; its request_reference is in the answer above."
    ))
}

/// `temper admin erasure resource` — survey (`--dry-run`) or erase one resource, optionally
/// striking related blobs from the survey's remainder.
pub async fn resource_remote(
    client: &temper_client::TemperClient,
    resource: &str,
    also_strike_blobs: Vec<uuid::Uuid>,
    dry_run: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let resource = parse_resource(resource)?;
    let admin = client.admin();
    if dry_run {
        let survey = admin
            .survey_resource_erasure(&ResourceErasureSurveyRequest { resource })
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return print(&survey, fmt);
    }
    let body = ResourceErasureExecuteRequest {
        resource,
        also_strike_blobs: (!also_strike_blobs.is_empty()).then_some(also_strike_blobs),
    };
    let answer = admin
        .erase_resource(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    print(&answer, fmt)?;
    match answer {
        ResourceErasureExecuteResponse::Completed { .. } => Ok(()),
        ResourceErasureExecuteResponse::Refused { reason, detail, .. } => {
            Err(refused("resource erasure", reason, detail.as_deref()))
        }
    }
}

/// `temper admin erasure principal` — survey (`--dry-run`) or erase a principal. Executing needs
/// the operator's opaque request reference; a survey requests nothing, so it takes none.
pub async fn principal_remote(
    client: &temper_client::TemperClient,
    subject: uuid::Uuid,
    request_reference: Option<uuid::Uuid>,
    dry_run: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let admin = client.admin();
    if dry_run {
        let survey = admin
            .survey_principal_erasure(&ErasureSurveyRequest { subject })
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return print(&survey, fmt);
    }
    // clap's `required_unless_present = "dry_run"` guarantees it here; refused rather than
    // unwrapped so the rule survives a caller that builds the arguments programmatically.
    let request_reference = request_reference.ok_or_else(|| {
        TemperError::BadRequest(
            "erasing a principal needs --request-reference (a survey with --dry-run does not)"
                .to_string(),
        )
    })?;
    let answer = admin
        .erase_principal(&ErasureExecuteRequest {
            subject,
            request_reference,
        })
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    print(&answer, fmt)
}

/// `temper admin erasure block-history` — survey (`--dry-run`) or scrub the named blocks'
/// revision history.
pub async fn block_history_remote(
    client: &temper_client::TemperClient,
    resource: &str,
    blocks: Vec<uuid::Uuid>,
    dry_run: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let body = BlockHistoryScrubRequestBody {
        resource: parse_resource(resource)?,
        blocks,
    };
    let admin = client.admin();
    if dry_run {
        let survey = admin
            .survey_block_history_scrub(&body)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return print(&survey, fmt);
    }
    let answer = admin
        .scrub_block_history(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    print(&answer, fmt)?;
    match answer {
        BlockHistoryScrubExecuteResponse::Completed { .. } => Ok(()),
        BlockHistoryScrubExecuteResponse::Refused { reason, detail, .. } => {
            Err(refused("block-history scrub", reason, detail.as_deref()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_names_the_act_and_the_wire_spelling_of_its_reason() {
        let err = refused(
            "resource erasure",
            ResourceErasureRefusalReason::CharterResource,
            Some("the resource is a map's charter"),
        );
        let TemperError::Conflict(message) = err else {
            panic!("a refusal is a Conflict");
        };
        assert!(message.starts_with(
            "resource erasure refused (charter_resource): the resource is a map's charter."
        ));
    }

    #[test]
    fn a_refusal_without_detail_reads_cleanly() {
        let TemperError::Conflict(message) = refused(
            "block-history scrub",
            ResourceErasureRefusalReason::AlreadyErased,
            None,
        ) else {
            panic!("a refusal is a Conflict");
        };
        assert!(message.starts_with("block-history scrub refused (already_erased). "));
    }

    /// A client whose server answers `body` on `route` (POST) and 404 everywhere else, so a test
    /// fails if the command reached for the wrong door.
    fn client_answering_on(
        route: &'static str,
        body: serde_json::Value,
    ) -> temper_client::TemperClient {
        let app = axum::Router::new().route(
            route,
            axum::routing::post(move || std::future::ready(axum::Json(body.clone()))),
        );
        temper_client::TemperClient::in_process_with_token(
            app,
            temper_workflow::operations::Surface::CliCloud,
            "tok".to_owned(),
            std::sync::Arc::new(temper_client::auth::MemoryTokenStore::empty()),
        )
        .expect("in-process client builds")
    }

    fn refused_answer() -> serde_json::Value {
        serde_json::json!({
            "status": "refused",
            "request_reference": uuid::Uuid::now_v7(),
            "event_id": uuid::Uuid::now_v7(),
            "reason": "ingest_in_flight",
            "detail": null,
        })
    }

    /// FAILS IF the refused arm is folded into success: a refused erasure must exit non-zero.
    #[tokio::test]
    async fn a_refused_resource_erasure_exits_as_a_conflict() {
        let client = client_answering_on("/api/admin/resources/erasure", refused_answer());
        let err = resource_remote(
            &client,
            &uuid::Uuid::now_v7().to_string(),
            vec![],
            false,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect_err("a refusal is not success");
        assert!(matches!(err, TemperError::Conflict(ref m) if m.contains("(ingest_in_flight)")));
    }

    #[tokio::test]
    async fn a_completed_resource_erasure_succeeds() {
        let client = client_answering_on(
            "/api/admin/resources/erasure",
            serde_json::json!({
                "status": "completed",
                "request_reference": uuid::Uuid::now_v7(),
                "event_id": uuid::Uuid::now_v7(),
                "folded_edges": [],
                "targets": [],
                "remainder": [],
                "ledger_remainder": [],
                "blob_strikes": [],
            }),
        );
        resource_remote(
            &client,
            &uuid::Uuid::now_v7().to_string(),
            vec![],
            false,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect("a completion is success");
    }

    /// `--dry-run` reaches the survey door and never the act: the stub mounts only the survey,
    /// so a stray execute would 404.
    #[tokio::test]
    async fn dry_run_surveys_and_never_executes() {
        let resource = uuid::Uuid::now_v7();
        let client = client_answering_on(
            "/api/admin/resources/erasure/survey",
            serde_json::json!({ "resource": resource, "already_erased": true, "plan": null }),
        );
        resource_remote(
            &client,
            &resource.to_string(),
            vec![],
            true,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect("the survey answers");
    }

    #[tokio::test]
    async fn a_refused_block_history_scrub_exits_as_a_conflict() {
        let mut answer = refused_answer();
        answer["blocks"] = serde_json::json!([]);
        let client = client_answering_on("/api/admin/resources/block-history-scrub", answer);
        let err = block_history_remote(
            &client,
            &uuid::Uuid::now_v7().to_string(),
            vec![uuid::Uuid::now_v7()],
            false,
            crate::format::OutputFormat::Json,
        )
        .await
        .expect_err("a refusal is not success");
        assert!(matches!(err, TemperError::Conflict(_)));
    }

    mod argv {
        use clap::{CommandFactory, Parser};

        use crate::cli::Cli;

        fn parses(args: &[&str]) -> bool {
            Cli::try_parse_from(args).is_ok()
        }

        #[test]
        fn erasing_a_principal_needs_a_request_reference_but_a_survey_does_not() {
            let subject = uuid::Uuid::now_v7().to_string();
            assert!(!parses(&[
                "temper",
                "admin",
                "erasure",
                "principal",
                &subject
            ]));
            assert!(parses(&[
                "temper",
                "admin",
                "erasure",
                "principal",
                &subject,
                "--dry-run"
            ]));
            let reference = uuid::Uuid::now_v7().to_string();
            assert!(parses(&[
                "temper",
                "admin",
                "erasure",
                "principal",
                &subject,
                "--request-reference",
                &reference,
            ]));
        }

        #[test]
        fn a_survey_cannot_name_blobs_to_strike() {
            let blob = uuid::Uuid::now_v7().to_string();
            assert!(!parses(&[
                "temper",
                "admin",
                "erasure",
                "resource",
                "r",
                "--also-strike-blob",
                &blob,
                "--dry-run",
            ]));
        }

        #[test]
        fn a_scrub_names_at_least_one_block() {
            assert!(!parses(&[
                "temper",
                "admin",
                "erasure",
                "block-history",
                "r"
            ]));
        }

        /// The bearer token rides the token store, never argv: no erasure command takes an
        /// argument that could carry it. FAILS IF a `--token`-like flag is added.
        #[test]
        fn no_erasure_command_takes_a_token_argument() {
            let cli = Cli::command();
            let erasure = cli
                .find_subcommand("admin")
                .and_then(|a| a.find_subcommand("erasure"))
                .expect("admin erasure exists");
            let mut acts = 0;
            for act in erasure.get_subcommands() {
                acts += 1;
                for arg in act.get_arguments() {
                    let id = arg.get_id().as_str().to_ascii_lowercase();
                    assert!(
                        !id.contains("token") && !id.contains("bearer") && !id.contains("secret"),
                        "`admin erasure {}` takes `{id}`, which could carry a credential",
                        act.get_name()
                    );
                }
            }
            assert_eq!(acts, 3, "resource, principal, block-history");
        }
    }
}

//! `temper admin erasure` — the erasure family's operator acts: erase a resource, erase a
//! principal, scrub a resource's block history. System admin only.
//!
//! Each act surveys by default: it asks the survey door what the act would do and records
//! nothing. Only `--execute` acts, and then once: the client never replays a write, and an erasure
//! is the last write that should be replayed. The survey is the default because the commonest
//! mistake is a forgotten flag, and a forgotten flag must never be the thing that erases.
//!
//! The answer is rendered in full, whatever the format: the per-target outcomes, the named
//! remainder, the ledger remainder, the refusal's reason and detail. A completed act and a refused
//! one read differently in two ways: the answer's `status` field, and the exit code — a refusal
//! prints its answer (it is the record of what happened) and then exits non-zero naming the reason,
//! so a script that drove the act cannot mistake a refusal for success.
//!
//! The bearer token comes from the token store, as for every command. Nothing here takes it as an
//! argument or prints it.

use std::io::Write;

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

fn emit<T: serde::Serialize>(
    value: &T,
    fmt: crate::format::OutputFormat,
    out: &mut impl Write,
) -> Result<()> {
    let rendered = crate::format::render(value, fmt)?;
    writeln!(out, "{rendered}").map_err(|e| TemperError::Api(format!("write answer: {e}")))
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

/// `temper admin erasure resource` — survey, or (`--execute`) erase one resource, optionally
/// striking related blobs from the survey's remainder.
pub async fn resource_remote(
    client: &temper_client::TemperClient,
    resource: &str,
    also_strike_blobs: Vec<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    resource_to(
        client,
        resource,
        also_strike_blobs,
        execute,
        fmt,
        &mut std::io::stdout(),
    )
    .await
}

async fn resource_to(
    client: &temper_client::TemperClient,
    resource: &str,
    also_strike_blobs: Vec<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
    out: &mut impl Write,
) -> Result<()> {
    let resource = parse_resource(resource)?;
    let admin = client.admin();
    if !execute {
        let survey = admin
            .survey_resource_erasure(&ResourceErasureSurveyRequest { resource })
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return emit(&survey, fmt, out);
    }
    let body = ResourceErasureExecuteRequest {
        resource,
        also_strike_blobs: (!also_strike_blobs.is_empty()).then_some(also_strike_blobs),
    };
    let answer = admin
        .erase_resource(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    emit(&answer, fmt, out)?;
    match answer {
        ResourceErasureExecuteResponse::Completed { .. } => Ok(()),
        ResourceErasureExecuteResponse::Refused { reason, detail, .. } => {
            Err(refused("resource erasure", reason, detail.as_deref()))
        }
    }
}

/// `temper admin erasure principal` — survey, or (`--execute`) erase a principal. Executing needs
/// the operator's opaque request reference; a survey requests nothing, so it takes none.
pub async fn principal_remote(
    client: &temper_client::TemperClient,
    subject: uuid::Uuid,
    request_reference: Option<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    principal_to(
        client,
        subject,
        request_reference,
        execute,
        fmt,
        &mut std::io::stdout(),
    )
    .await
}

async fn principal_to(
    client: &temper_client::TemperClient,
    subject: uuid::Uuid,
    request_reference: Option<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
    out: &mut impl Write,
) -> Result<()> {
    let admin = client.admin();
    if !execute {
        let survey = admin
            .survey_principal_erasure(&ErasureSurveyRequest { subject })
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return emit(&survey, fmt, out);
    }
    // clap's `requires = "request_reference"` on --execute guarantees it here; refused rather
    // than unwrapped so the rule survives a caller that builds the arguments programmatically.
    let request_reference = request_reference.ok_or_else(|| {
        TemperError::BadRequest(
            "erasing a principal needs --request-reference (a survey does not)".to_string(),
        )
    })?;
    let answer = admin
        .erase_principal(&ErasureExecuteRequest {
            subject,
            request_reference,
        })
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    emit(&answer, fmt, out)
}

/// `temper admin erasure block-history` — survey, or (`--execute`) scrub the named blocks'
/// revision history.
pub async fn block_history_remote(
    client: &temper_client::TemperClient,
    resource: &str,
    blocks: Vec<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    block_history_to(
        client,
        resource,
        blocks,
        execute,
        fmt,
        &mut std::io::stdout(),
    )
    .await
}

async fn block_history_to(
    client: &temper_client::TemperClient,
    resource: &str,
    blocks: Vec<uuid::Uuid>,
    execute: bool,
    fmt: crate::format::OutputFormat,
    out: &mut impl Write,
) -> Result<()> {
    let body = BlockHistoryScrubRequestBody {
        resource: parse_resource(resource)?,
        blocks,
    };
    let admin = client.admin();
    if !execute {
        let survey = admin
            .survey_block_history_scrub(&body)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        return emit(&survey, fmt, out);
    }
    let answer = admin
        .scrub_block_history(&body)
        .await
        .map_err(crate::actions::runtime::client_err_to_temper)?;
    emit(&answer, fmt, out)?;
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

    /// A token no rendered answer could contain by accident, so its absence means something.
    const TOKEN: &str = "tok-7f3a-never-rendered";

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
            TOKEN.to_owned(),
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
            "detail": "an ingest holds the resource",
        })
    }

    fn completed_answer() -> serde_json::Value {
        serde_json::json!({
            "status": "completed",
            "request_reference": uuid::Uuid::now_v7(),
            "event_id": uuid::Uuid::now_v7(),
            "folded_edges": [],
            "targets": [{ "target": "kb_blocks", "outcome": "erased" }],
            "remainder": [{ "target": "kb_blobs", "outcome": "named-unreached" }],
            "redacted_fields": [{ "event": uuid::Uuid::now_v7(), "paths": ["title"] }],
            "ledger_remainder": [{ "event": uuid::Uuid::now_v7(), "paths": ["telos_centroid"] }],
            "blob_strikes": [],
        })
    }

    const FORMATS: [crate::format::OutputFormat; 2] = [
        crate::format::OutputFormat::Json,
        crate::format::OutputFormat::Toon,
    ];

    /// FAILS IF the refused arm is folded into success: a refused erasure must exit non-zero, and
    /// its answer — reason and detail — is printed first, in either format, without the token.
    #[tokio::test]
    async fn a_refused_resource_erasure_prints_its_answer_and_exits_as_a_conflict() {
        for fmt in FORMATS {
            let client = client_answering_on("/api/admin/resources/erasure", refused_answer());
            let mut out = Vec::new();
            let err = resource_to(
                &client,
                &uuid::Uuid::now_v7().to_string(),
                vec![],
                true,
                fmt,
                &mut out,
            )
            .await
            .expect_err("a refusal is not success");
            assert!(
                matches!(err, TemperError::Conflict(ref m) if m.contains("(ingest_in_flight)"))
            );
            let out = String::from_utf8(out).expect("utf-8");
            for needle in [
                "refused",
                "ingest_in_flight",
                "an ingest holds the resource",
            ] {
                assert!(
                    out.contains(needle),
                    "{fmt:?} answer lacks {needle:?}:\n{out}"
                );
            }
            assert!(!out.contains(TOKEN), "{fmt:?} answer carries the token");
            assert!(
                !err.to_string().contains(TOKEN),
                "the refusal carries the token"
            );
        }
    }

    /// FAILS IF a completed act's answer drops what it reached or left: targets, remainder, the
    /// redacted ledger paths and the ledger remainder are all rendered, in either format, and the
    /// token is not.
    #[tokio::test]
    async fn a_completed_resource_erasure_renders_targets_remainder_and_ledger_remainder() {
        for fmt in FORMATS {
            let client = client_answering_on("/api/admin/resources/erasure", completed_answer());
            let mut out = Vec::new();
            resource_to(
                &client,
                &uuid::Uuid::now_v7().to_string(),
                vec![],
                true,
                fmt,
                &mut out,
            )
            .await
            .expect("a completion is success");
            let out = String::from_utf8(out).expect("utf-8");
            for needle in [
                "completed",
                "targets",
                "kb_blocks",
                "remainder",
                "kb_blobs",
                "redacted_fields",
                "title",
                "ledger_remainder",
                "telos_centroid",
            ] {
                assert!(
                    out.contains(needle),
                    "{fmt:?} answer lacks {needle:?}:\n{out}"
                );
            }
            assert!(!out.contains(TOKEN), "{fmt:?} answer carries the token");
        }
    }

    /// Without `--execute` the command reaches the survey door and never the act: the stub mounts
    /// only the survey, so a stray execute would 404.
    #[tokio::test]
    async fn without_execute_it_surveys_and_never_acts() {
        let resource = uuid::Uuid::now_v7();
        let client = client_answering_on(
            "/api/admin/resources/erasure/survey",
            serde_json::json!({ "resource": resource, "already_erased": true, "plan": null }),
        );
        let mut out = Vec::new();
        resource_to(
            &client,
            &resource.to_string(),
            vec![],
            false,
            crate::format::OutputFormat::Json,
            &mut out,
        )
        .await
        .expect("the survey answers");
        assert!(!String::from_utf8(out).expect("utf-8").contains(TOKEN));
    }

    #[tokio::test]
    async fn a_refused_block_history_scrub_exits_as_a_conflict() {
        let mut answer = refused_answer();
        answer["blocks"] = serde_json::json!([]);
        let client = client_answering_on("/api/admin/resources/block-history-scrub", answer);
        let err = block_history_to(
            &client,
            &uuid::Uuid::now_v7().to_string(),
            vec![uuid::Uuid::now_v7()],
            true,
            crate::format::OutputFormat::Json,
            &mut Vec::new(),
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

        fn act(args: &[&str]) -> Option<crate::cli::AdminErasureAction> {
            let cli = Cli::try_parse_from(args).ok()?;
            match cli.command {
                crate::cli::Commands::Admin {
                    action: crate::cli::AdminAction::Erasure { action },
                } => Some(action),
                _ => None,
            }
        }

        /// FAILS IF a bare command can act: with no flag, every act only surveys.
        #[test]
        fn a_bare_command_only_surveys() {
            let id = uuid::Uuid::now_v7().to_string();
            let bare = [
                vec!["temper", "admin", "erasure", "resource", id.as_str()],
                vec!["temper", "admin", "erasure", "principal", id.as_str()],
                vec![
                    "temper",
                    "admin",
                    "erasure",
                    "block-history",
                    id.as_str(),
                    "--block",
                    id.as_str(),
                ],
            ];
            for args in bare {
                use crate::cli::AdminErasureAction as A;
                let execute = match act(&args).expect("a bare survey parses") {
                    A::Resource { execute, .. }
                    | A::Principal { execute, .. }
                    | A::BlockHistory { execute, .. } => execute,
                };
                assert!(!execute, "`{}` would act", args.join(" "));
            }
        }

        #[test]
        fn erasing_a_principal_needs_a_request_reference_and_a_survey_takes_none() {
            let subject = uuid::Uuid::now_v7().to_string();
            let reference = uuid::Uuid::now_v7().to_string();
            let base = ["temper", "admin", "erasure", "principal", subject.as_str()];
            let with = |extra: &[&str]| parses(&[&base[..], extra].concat());
            assert!(!with(&["--execute"]), "--execute without a reference");
            assert!(
                !with(&["--request-reference", &reference]),
                "a reference on a survey"
            );
            assert!(with(&["--execute", "--request-reference", &reference]));
        }

        #[test]
        fn only_an_execute_can_name_blobs_to_strike() {
            let blob = uuid::Uuid::now_v7().to_string();
            let base = ["temper", "admin", "erasure", "resource", "r"];
            let with = |extra: &[&str]| parses(&[&base[..], extra].concat());
            assert!(!with(&["--also-strike-blob", &blob]));
            assert!(with(&["--also-strike-blob", &blob, "--execute"]));
        }

        #[test]
        fn a_scrub_names_at_least_one_block() {
            assert!(!parses(&[
                "temper",
                "admin",
                "erasure",
                "block-history",
                "r",
                "--execute"
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
